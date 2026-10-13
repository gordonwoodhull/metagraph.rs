//! M4 step 12 — phased stores: the Tx over a FROZEN phased store flushes
//! into a fresh `Store<Build>` — kept membership carried (the csr walk
//! feeds the new store's buckets from the old store's frozen ranges, the
//! intrusive carry rebuilds by walk), staged ops replayed through the
//! real constructors, and the pipeline ending in `finish()` (freeze, then
//! validate). These are the generated equivalents of
//! /tmp/m4-probe-phased-flush's 12 tests (its REPORT.md pins the
//! semantics), recast over the stdlib `Digraph` with `out_edges:
//! csr_adjacency` + `in_edges: intrusive_list` — the probe's exact shape.
//!
//! Probe vocabulary maps one-to-one here: GraphStore<Frozen> is the old
//! store, `node`/`edge` arenas, `offsets`/`adj` the frozen CSR fields,
//! `next_in` the intrusive link. The tests live INSIDE the `implement!`ing
//! module so the corruption tests can reach the private fields (the
//! csr.rs driver shape).

mod mixed {
    use metagraph::implement;
    use metagraph::stdlib::{InEdges, Nodes, OutEdges, Source, Target};

    implement! {
        metagraph::stdlib::Digraph as GraphStore {
            nodes:     arena,
            out_edges: csr_adjacency,
            in_edges:  intrusive_list,
        }
    }

    /// The probe's shared old store: nodes n0..n4; e0:n0->n1 e1:n1->n2
    /// e2:n2->n3 e3:n3->n4 e4:n0->n2 e5:n1->n4 e6:n2->n4.
    fn build_old() -> GraphStore<Frozen> {
        let mut b = GraphStore::new();
        let n0 = b.add_node();
        let n1 = b.add_node();
        let n2 = b.add_node();
        let n3 = b.add_node();
        let n4 = b.add_node();
        assert_eq!(
            (n0, n1, n2, n3, n4),
            (
                NodeId(0),
                NodeId(1),
                NodeId(2),
                NodeId(3),
                NodeId(4)
            )
        );
        let e0 = b.add_edge(n0, n1);
        let e1 = b.add_edge(n1, n2);
        let e2 = b.add_edge(n2, n3);
        let e3 = b.add_edge(n3, n4);
        let e4 = b.add_edge(n0, n2);
        let e5 = b.add_edge(n1, n4);
        let e6 = b.add_edge(n2, n4);
        assert_eq!(
            (e0, e1, e2, e3, e4, e5, e6),
            (
                EdgeId(0),
                EdgeId(1),
                EdgeId(2),
                EdgeId(3),
                EdgeId(4),
                EdgeId(5),
                EdgeId(6)
            )
        );
        b.finish().expect("old store valid")
    }

    /// The probe's shared happy-path change set. Returns the pending node
    /// ref and the two staged edge refs.
    fn stage_happy(tx: &mut Tx<'_>) -> (NodeRef, EdgeRef, EdgeRef) {
        tx.remove_edge(EdgeId(4)); // op0: one middle edge
        tx.remove_edge(EdgeId(0)); // op1: n1's incident edges ...
        tx.remove_edge(EdgeId(1)); // op2
        tx.remove_edge(EdgeId(5)); // op3
        tx.remove_node(NodeId(1)); // op4: ... then the node
        let p = tx.add_node(); // op5
        let ea = tx.add_edge(NodeId(0), p); // op6: Existing -> Pending
        let eb = tx.add_edge(p, NodeId(4)); // op7: Pending -> Existing
        (p, ea, eb)
    }

    /// Happy-path flush, for tests that just need the outcome.
    fn flush_happy() -> (GraphStore<Frozen>, FlushReport) {
        let old = build_old();
        let mut tx = old.edit();
        stage_happy(&mut tx);
        let out = tx.flush().expect("happy-path flush");
        (out.store, out.report)
    }

    fn node_at<'g>(store: &'g GraphStore<Frozen>, id: NodeId) -> Node<'g> {
        store.graph().nodes().find(|x| x.id() == id).unwrap()
    }

    fn ids<'a>(it: impl Iterator<Item = Edge<'a>>) -> Vec<EdgeId> {
        it.map(|e| e.id()).collect()
    }

    // -- Assertion 1: the happy-path change set flushes -------------------

    #[test]
    fn happy_path_flush_kept_and_staged() {
        let (f, rep) = flush_happy();
        assert_eq!(f.node_count(), 5); // 4 kept + 1 staged
        assert_eq!(f.edge_count(), 5); // 3 kept + 2 staged
        // n1 removed; kept nodes compacted in old order; staged node last.
        assert_eq!(rep.node_map, vec![Some(0), None, Some(1), Some(2), Some(3)]);
        // e0,e1,e4,e5 removed; kept edges compacted in old order.
        assert_eq!(
            rep.edge_map,
            vec![None, None, Some(0), Some(1), None, None, Some(2)]
        );
        assert_eq!(f.validate(), ::std::result::Result::Ok(()));
    }

    // -- Assertion 2: CSR out-ranges == kept edges in the OLD frozen
    //    forward order, compacted -----------------------------------------

    #[test]
    fn csr_out_ranges_carry_kept_forward_order() {
        let old = build_old();
        let (f, rep) = flush_happy();

        // Recompute the expectation independently: walk the OLD frozen CSR
        // (node arena order, then each node's adj range), keep+remap.
        let exp_adj: Vec<EdgeId>;
        let mut exp_offsets = vec![0u32];
        // kept nodes in old arena order == new-id order (step-3 compaction)
        let kept: Vec<usize> = (0..old.node_count())
            .filter(|&i| rep.node_map[i].is_some())
            .collect();
        // per-node adj segments, kept edges first (old frozen forward
        // order), then staged edges appended by replay (op6 -> new node 0,
        // op7 -> new node 4; staged edge ids run kept_edge_count.. in
        // AddEdge submission order). Segment count = flushed node count =
        // kept + the one staged AddNode (op5), whose own bucket takes op7.
        let mut segments: Vec<Vec<EdgeId>> = vec![Vec::new(); kept.len() + 1];
        for (k, &old_nid) in kept.iter().enumerate() {
            let lo = old.offsets[old_nid] as usize;
            let hi = old.offsets[old_nid + 1] as usize;
            for &e in &old.adj[lo..hi] {
                if let Some(ne) = rep.edge_map[e.0 as usize] {
                    segments[k].push(EdgeId(ne));
                }
            }
        }
        for (src, e) in [(0u32, 3u32), (4, 4)] {
            segments[src as usize].push(EdgeId(e));
        }
        exp_adj = segments.concat();
        let mut run = 0u32;
        for seg in &segments {
            run += seg.len() as u32;
            exp_offsets.push(run);
        }
        assert_eq!(f.offsets, exp_offsets, "flushed offsets");
        assert_eq!(f.adj, exp_adj, "flushed adj");

        // And through the public handle: per kept node, out-edges equal the
        // compacted old forward order (+ nothing else for these nodes).
        let expect_out = |new_nid: u32| -> Vec<EdgeId> {
            exp_adj[exp_offsets[new_nid as usize] as usize
                ..exp_offsets[new_nid as usize + 1] as usize]
                .to_vec()
        };
        for old_nid in 0..old.node_count() {
            if let Some(&new_nid) = rep.node_map[old_nid].as_ref() {
                assert_eq!(
                    ids(node_at(&f, NodeId(new_nid)).out_edges()),
                    expect_out(new_nid),
                    "node old={old_nid} new={new_nid}"
                );
            }
        }
        // Spot-check the interesting node: old n2 kept e2,e6, dropped middle
        // neighbor-edge e4 -> bucket [0, 2], no reorder.
        assert_eq!(
            ids(node_at(&f, NodeId(1)).out_edges()),
            vec![EdgeId(0), EdgeId(2)]
        );
    }

    // -- Assertion 3: in-lists == rebuild-by-walk of the old chains,
    //    staged inserts iterating FIRST on their targets ------------------

    #[test]
    fn intrusive_in_lists_rebuilt_by_walk_then_staged_first() {
        let old = build_old();
        let (f, rep) = flush_happy();

        for old_nid in 0..old.node_count() {
            let Some(&new_nid) = rep.node_map[old_nid].as_ref() else {
                continue;
            };
            // Walk the OLD chain head->tail (reverse-insertion order), keep
            // and remap: the required relative order for kept edges.
            let mut walked: Vec<EdgeId> = Vec::new();
            let mut cur = old.node_arena[old_nid].first_in;
            while let Some(e) = cur {
                cur = old.edge_arena[e.0 as usize].next_in;
                if let Some(ne) = rep.edge_map[e.0 as usize] {
                    walked.push(EdgeId(ne));
                }
            }
            // Staged edges targeting this node iterate FIRST (push-front),
            // in reverse replay order. Staged ids: op6 -> 3 (target = the
            // new node 4), op7 -> 4 (target = new node 3).
            let staged_by_target = [(4u32, 3u32), (3, 4)];
            let onto: Vec<EdgeId> = staged_by_target
                .iter()
                .rev()
                .filter(|(t, _)| *t == new_nid)
                .map(|(_, e)| EdgeId(*e))
                .collect();
            let expected = [onto, walked].concat();
            assert_eq!(
                ids(node_at(&f, NodeId(new_nid)).in_edges()),
                expected,
                "node old={old_nid} new={new_nid}"
            );
        }
        // The two staged inserts iterate first on their target in-lists.
        assert_eq!(
            ids(node_at(&f, NodeId(3)).in_edges()),
            vec![EdgeId(4), EdgeId(2), EdgeId(1)]
        );
        assert_eq!(ids(node_at(&f, NodeId(4)).in_edges()), vec![EdgeId(3)]);
        // Old n4's chain was e6 -> e5 -> e3; e5 removed in the MIDDLE, and
        // the walk skipped it without truncating e3 off the tail.
        assert_eq!(
            ids(node_at(&f, NodeId(3)).in_edges())[1..],
            [EdgeId(2), EdgeId(1)][..]
        );
    }

    // -- Assertion 4: signature equality with direct construction ---------

    #[test]
    fn signature_equality_with_direct_construction() {
        let (flushed, _) = flush_happy();

        // The equivalent final graph built DIRECTLY. Normalization contract:
        //   * nodes: the direct build creates kept nodes in old-arena order,
        //     then the staged node — the same order the flush produces
        //     (kept records copied first, staged AddNode replayed last), so
        //     flushed node i and direct node di are the same logical node;
        //   * edges: kept edges inserted in ASCENDING OLD-ID order (which is
        //     simultaneously the old per-source and per-target forward
        //     insertion order), then the staged AddEdges in submission
        //     order. Edge ids are NOT assumed to correspond in general; the
        //     comparison maps both sides to (source, target) arena-index
        //     pairs in each backing's iteration order.
        let mut d = GraphStore::new();
        let d0 = d.add_node();
        let d1 = d.add_node();
        let d2 = d.add_node();
        let d3 = d.add_node();
        let d4 = d.add_node();
        let _ = d.add_edge(d1, d2); // old e2
        let _ = d.add_edge(d2, d3); // old e3
        let _ = d.add_edge(d1, d3); // old e6
        let _ = d.add_edge(d0, d4); // op6
        let _ = d.add_edge(d4, d3); // op7
        let direct = d.finish().expect("direct build valid");

        assert_eq!(flushed.node_count(), direct.node_count());
        assert_eq!(flushed.edge_count(), direct.edge_count());

        let sig = |s: &GraphStore<Frozen>| -> (Vec<Vec<(u32, u32)>>, Vec<Vec<(u32, u32)>>) {
            let out: Vec<Vec<(u32, u32)>> = s
                .graph()
                .nodes()
                .map(|n| {
                    n.out_edges()
                        .map(|e| (e.source().id().0, e.target().id().0))
                        .collect()
                })
                .collect();
            let inn: Vec<Vec<(u32, u32)>> = s
                .graph()
                .nodes()
                .map(|n| {
                    n.in_edges()
                        .map(|e| (e.source().id().0, e.target().id().0))
                        .collect()
                })
                .collect();
            (out, inn)
        };
        assert_eq!(sig(&flushed), sig(&direct), "structural signature");

        // Observation, not an assumption: for THIS change set the flushed
        // and direct edge ids also coincide, because both sides order edges
        // ascending-old-id-then-submission. The comparison above does not
        // rely on it.
        for i in 0..direct.edge_count() {
            let fe = &flushed.edge_arena[i];
            let de = &direct.edge_arena[i];
            assert_eq!((fe.source, fe.target), (de.source, de.target));
        }
    }

    // -- Assertion 5: staged edge onto a removed node ---------------------

    #[test]
    fn staged_edge_onto_removed_node_fails_with_op_index() {
        let old = build_old();
        let mut tx = old.edit();
        tx.remove_node(NodeId(1)); // op0
        tx.add_edge(NodeId(0), NodeId(1)); // op1: onto the removed node
        let err = tx.flush().expect_err("staged dangler must fail");
        // The staged AddEdge fails step 2 with ITS OWN op index and the old
        // id it named, listed after the kept incident-edge danglers of n1
        // (attributed to the removal op).
        assert_eq!(
            err.violations,
            vec![
                Violation::DanglingEndpoint {
                    op: 0,
                    record: RecordDesc::Kept { arena: "edge", id: 0 },
                    endpoint_arena: "node",
                    endpoint: 1,
                },
                Violation::DanglingEndpoint {
                    op: 0,
                    record: RecordDesc::Kept { arena: "edge", id: 1 },
                    endpoint_arena: "node",
                    endpoint: 1,
                },
                Violation::DanglingEndpoint {
                    op: 0,
                    record: RecordDesc::Kept { arena: "edge", id: 5 },
                    endpoint_arena: "node",
                    endpoint: 1,
                },
                Violation::DanglingEndpoint {
                    op: 1,
                    record: RecordDesc::Staged(1),
                    endpoint_arena: "node",
                    endpoint: 1,
                },
            ]
        );
        assert!(
            err.to_string().contains("DanglingEndpoint"),
            "display: {err}"
        );
        // (The generated Display prints raw ids — `endpoint: 1` — where
        // the probe's printed `NodeId(1)`; the structured vector above is
        // the load-bearing assertion.)
    }

    // -- Assertion 6: kept dangler attributed to the removal op -----------

    #[test]
    fn kept_dangler_attributed_to_removal_op() {
        let old = build_old();
        let mut tx = old.edit();
        tx.remove_node(NodeId(2)); // op0: incident edges e1,e2,e4,e6 NOT removed
        let err = tx.flush().expect_err("danglers must be rejected");
        assert_eq!(err.violations.len(), 4, "one dangler per incident kept edge");
        assert_eq!(
            err.violations,
            vec![
                Violation::DanglingEndpoint {
                    op: 0,
                    record: RecordDesc::Kept { arena: "edge", id: 1 },
                    endpoint_arena: "node",
                    endpoint: 2,
                },
                Violation::DanglingEndpoint {
                    op: 0,
                    record: RecordDesc::Kept { arena: "edge", id: 2 },
                    endpoint_arena: "node",
                    endpoint: 2,
                },
                Violation::DanglingEndpoint {
                    op: 0,
                    record: RecordDesc::Kept { arena: "edge", id: 4 },
                    endpoint_arena: "node",
                    endpoint: 2,
                },
                Violation::DanglingEndpoint {
                    op: 0,
                    record: RecordDesc::Kept { arena: "edge", id: 6 },
                    endpoint_arena: "node",
                    endpoint: 2,
                },
            ]
        );
    }

    // -- Assertion 7: finish() passes on the flushed store; validate
    //    catches post-freeze damage ---------------------------------------

    #[test]
    fn finish_passes_and_validate_catches_damage() {
        // The flush pipeline ENDS in finish() (freeze, then validate), so a
        // flushed store is born valid; assert it again on the outcome.
        let (f, _) = flush_happy();
        assert_eq!(f.validate(), ::std::result::Result::Ok(()));

        // Corrupt an intrusive link post-freeze -> validate must name the
        // offending edge. (Edge 3 = the staged op6 edge.)
        let (mut t, _) = flush_happy();
        t.edge_arena[3].next_in = Some(EdgeId(99));
        let err = t.validate().expect_err("intrusive damage must fail");
        assert!(err.contains("EdgeId(99)"), "message was: {err}");

        // ... and a CSR poke trips the CSR validator (a +1 that stays
        // monotonic would rightly pass; push offsets[3] above offsets[4]).
        let (mut t, _) = flush_happy();
        t.offsets[3] += 1; // [0,1,3,5,4,5] -> not monotonic
        let err = t.validate().expect_err("csr damage must fail");
        assert!(err.contains("offsets"), "message was: {err}");
    }

    // -- Bonus findings ----------------------------------------------------

    /// The probe's finding c.2 (validate cannot backstop a wrong carry)
    /// splits in two against the GENERATED validators, which are stronger
    /// than the probe's hand-written structural checks in one direction
    /// and no stronger in the other:
    ///
    /// - The generated intrusive validator runs M2's partition walk (every
    ///   member must appear in its target's list), so the probe's exact
    ///   truncation poke — dropping edge 1 off node 3's in-list tail — IS
    ///   caught, naming the missing edge. (Per-field link remapping would
    ///   still be silently wrong mid-chain; assertion 3's middle-removal
    ///   case is the load-bearing guard.)
    /// - The generated CSR validator checks only the offsets invariants
    ///   (length, span, monotonicity) — NOT membership or order. Swapping
    ///   two adj entries inside one node's range is semantic reorder
    ///   damage validate cannot see: the probe's lesson, in generated
    ///   form. Signature assertions are the only guard.
    #[test]
    fn validate_catches_truncation_but_misses_reorder() {
        // Truncation: node 3's in-list is [4, 2, 1]; drop the 1 off the
        // tail. The partition walk names the missing edge.
        let (mut f, _) = flush_happy();
        f.edge_arena[2].next_in = None;
        let err = f.validate().expect_err("truncation must fail");
        assert!(err.contains("EdgeId(1)"), "message was: {err}");

        // Reorder: node 1's out-range is adj[1..3] = [0, 2]; swap the
        // entries. Offsets stay monotonic, adj length unchanged — the CSR
        // validator passes, but the iteration order changed.
        let (mut f, _) = flush_happy();
        assert_eq!(f.adj, vec![EdgeId(3), EdgeId(0), EdgeId(2), EdgeId(1), EdgeId(4)]);
        f.adj.swap(1, 2);
        assert_eq!(
            f.validate(),
            ::std::result::Result::Ok(()),
            "structural csr validators all pass"
        );
        assert_eq!(
            ids(node_at(&f, NodeId(1)).out_edges()),
            vec![EdgeId(2), EdgeId(0)]
        );
    }

    #[test]
    fn unknown_old_id_and_double_removal_fail() {
        let old = build_old();
        let mut tx = old.edit();
        tx.remove_edge(EdgeId(99)); // op0: unknown old id
        let err = tx.flush().expect_err("unknown old id must fail");
        assert!(matches!(
            &err.violations[0],
            Violation::UnknownOldId { op: 0, arena: "edge", id: 99 }
        ));

        let old = build_old();
        let mut tx = old.edit();
        tx.remove_node(NodeId(3)); // op0
        tx.remove_node(NodeId(3)); // op1: double removal
        let err = tx.flush().expect_err("double removal must fail");
        assert!(matches!(
            &err.violations[0],
            Violation::DoubleRemoval { op: 1, arena: "node", id: 3 }
        ));
    }

    #[test]
    fn empty_change_set_flush_is_identity() {
        let old = build_old();
        let out = old.edit().flush().expect("empty tx flushes");
        assert_eq!(
            out.report.node_map,
            vec![Some(0), Some(1), Some(2), Some(3), Some(4)]
        );
        assert_eq!(
            out.report.edge_map,
            vec![
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6)
            ]
        );
        for i in 0..old.node_count() {
            let id = NodeId(i as u32);
            assert_eq!(
                ids(node_at(&old, id).out_edges()),
                ids(node_at(&out.store, id).out_edges())
            );
            assert_eq!(
                ids(node_at(&old, id).in_edges()),
                ids(node_at(&out.store, id).in_edges())
            );
        }
    }

    #[test]
    #[should_panic(expected = "different transaction token")]
    fn pending_ref_from_another_tx_panics_at_staging() {
        let old = build_old();
        let mut tx1 = old.edit();
        let p = tx1.add_node();
        let mut tx2 = old.edit();
        // Eager stamp check fires HERE, at staging — not at flush.
        tx2.add_edge(NodeId(0), p);
    }

    #[test]
    #[should_panic(expected = "no old record to remove")]
    fn removing_a_pending_ref_panics_at_staging() {
        let old = build_old();
        let mut tx = old.edit();
        let p = tx.add_node();
        tx.remove_node(p);
    }
}
