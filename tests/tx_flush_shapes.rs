//! M4 step 3 — flush smokes on the other emitted shapes: the step's main
//! acceptance (tests/tx_flush.rs) drives the by-derived Digraph; these
//! drive the two remaining carry placements the step-3 gate emits — a
//! primitive (non-`by`) intrusive list at an element self, and a
//! root-sourced list whose head folds to a store field (one chain for the
//! whole store). Compile-only until now; each smoke flushes and checks the
//! carried membership. Each shape's `implement!` emits its own `Tx`, so
//! each module carries its own tap.

/// A per-module flush tap: stage, then flush, in one expression (spike 7's
/// `tap_flush`, instantiated against each module's generated `Tx`).
macro_rules! with_tap {
    ($mod:ident, $test_mod:ident, { $($body:tt)* }) => {
        mod $test_mod {
            use super::$mod::*;
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
            $($body)*
        }
    };
}

// ---- primitive intrusive list + field-backed `one` arrows (CsrDigraph) --

mod prim {
    use metagraph::implement;
    use metagraph::stdlib::{Nodes, OutEdges};

    implement! {
        m2_fixtures::shapes::CsrDigraph as PrimLinks {
            nodes:  arena,
            out:    intrusive_list,
            // `source`/`target` are `one` arrows: field-backed by default.
        }
    }

    /// A node's out-list as new-id sequence (primitive membership rides the
    /// `out` list, iterated in reverse insertion order).
    pub(super) fn out_ids(s: &PrimLinks, n: NId) -> Vec<u32> {
        let node = s.g().nodes().find(|x| x.id() == n).unwrap();
        node.out_edges().map(|e| e.id().0).collect()
    }
}

with_tap!(prim, prim_tests, {
    use super::prim::PrimLinks;

    #[test]
    fn prim_list_carry_and_field_remap() {
        let mut base = PrimLinks::new();
        let a = base.add_n();
        let b = base.add_n();
        let e0 = base.add_e(a, b);
        let e1 = base.add_e(a, b);
        let e2 = base.add_e(a, b);
        // Explicit inserts (primitive membership): a's list iterates [e2, e1, e0].
        base.insert_out(a, e0);
        base.insert_out(a, e1);
        base.insert_out(a, e2);
        assert_eq!(out_ids(&base, a), vec![2, 1, 0]);

        // Remove the middle of the chain and stage a genuinely new edge: the
        // walk must splice around e1 (kept order preserved), the field-backed
        // endpoints remap, and the new record appends after the kept ones.
        let out = base
            .edit()
            .tap_flush(|tx| {
                tx.remove_e(EId(1));
                let x = tx.add_n();
                tx.add_e(x, a);
            })
            .expect("flush");

        assert_eq!(out.store.n_count(), 3);
        assert_eq!(out.store.e_count(), 3);
        // e0 kept -> 0, e1 removed, e2 kept -> 1; the staged edge is new id 2.
        assert_eq!(out.report.e_map, vec![Some(0), None, Some(1)]);
        assert_eq!(out.report.n_map, vec![Some(0), Some(1)]);
        // The walk spliced e1 out: a's list is [e2', e0'] = [1, 0].
        assert_eq!(out_ids(&out.store, NId(0)), vec![1, 0]);
        // The staged edge's endpoints resolved through map + pending table.
        assert_eq!(out.store.source(EId(2)), NId(2));
        assert_eq!(out.store.target(EId(2)), NId(0));
        assert_eq!(out_ids(&out.store, NId(2)), vec![]); // primitive: born unlisted
        out.store.validate().expect("validate");
        // Old store untouched.
        assert_eq!(out_ids(&base, a), vec![2, 1, 0]);
    }

    #[test]
    fn prim_field_danglers_on_removal() {
        let mut base = PrimLinks::new();
        let a = base.add_n();
        let b = base.add_n();
        let _ = base.add_e(a, b);

        // Removing a node without its edges: the field-backed `source`/`target`
        // one-arrows dangle, attributed to the removal op.
        let err = base
            .edit()
            .tap_flush(|tx| tx.remove_n(NId(0)))
            .expect_err("danglers");
        assert_eq!(err.violations.len(), 1);
        assert_eq!(
            err.violations[0],
            Violation::DanglingEndpoint {
                op: 0,
                record: RecordDesc::Kept { arena: "e", id: 0 },
                endpoint_arena: "n",
                endpoint: 0,
            }
        );

        // A staged add naming the removed node dangles at its own op —
        // beside the kept e0, which dangles for the plain reason
        // (attributed to the removal).
        let err = base
            .edit()
            .tap_flush(|tx| {
                tx.remove_n(NId(0));
                let x = tx.add_n();
                tx.add_e(x, NId(0));
            })
            .expect_err("staged dangler");
        assert_eq!(
            err.violations,
            vec![
                Violation::DanglingEndpoint {
                    op: 0,
                    record: RecordDesc::Kept { arena: "e", id: 0 },
                    endpoint_arena: "n",
                    endpoint: 0,
                },
                Violation::DanglingEndpoint {
                    op: 2, // remove_n=0, add_n=1, add_e=2
                    record: RecordDesc::Staged(2),
                    endpoint_arena: "n",
                    endpoint: 0,
                },
            ]
        );
    }

    #[test]
    fn prim_insert_through_tx_replays_on_top_of_carry() {
        // M4 step 6: the `insert_out` staging verb (and its root-fixed
        // trait spelling) queues an Insert op; the flush replays it
        // through the backing's insert hook AFTER the carried skeleton,
        // so an inserted member push-fronts onto the carried list.
        // Members must be UNLISTED (kept-but-unlisted or staged): like
        // the raw store op, re-inserting a listed member corrupts the
        // chain — the Tx does not guard duplicates.
        use metagraph::stdlib::InsertOutEdges;

        let mut base = PrimLinks::new();
        let a = base.add_n();
        let b = base.add_n();
        let e0 = base.add_e(a, b);
        let e1 = base.add_e(a, b); // kept, never listed
        base.insert_out(a, e0); // a's list: [e0]

        // Inherent spelling: a staged (Pending) member insert.
        let out = base
            .edit()
            .tap_flush(|tx| {
                let e2 = tx.add_e(b, a); // born unlisted (step-3 note d)
                tx.insert_out(a, e2);
            })
            .expect("flush");
        // Carried [e0'], then the staged e2' push-fronts: [e2', e0'].
        // (The report maps are old-id indexed; the staged edge has no old id.)
        assert_eq!(out_ids(&out.store, NId(0)), vec![2, 0]);
        out.store.validate().expect("validate");

        // Trait spelling (root-fixed name, same op): an existing
        // kept-but-unlisted member insert. The trait method takes the
        // ref types (canonical arity); raw ids convert via `existing`.
        let out2 = base
            .edit()
            .tap_flush(|tx| {
                InsertOutEdges::insert_out_edges(
                    &mut *tx,
                    super::prim::NRef::existing(a),
                    super::prim::ERef::existing(e1),
                );
            })
            .expect("trait flush");
        assert_eq!(out_ids(&out2.store, NId(0)), vec![1, 0]);
    }

    #[test]
    fn prim_insert_naming_removed_record_is_a_dangler() {
        // A staged insert naming a removed host or member is a
        // DanglingEndpoint attributed to the insert op (a removed record
        // cannot accept membership), not a replay-time panic.
        let mut base = PrimLinks::new();
        let a = base.add_n();
        let b = base.add_n();
        let e0 = base.add_e(a, b);
        base.insert_out(a, e0);

        let err = base
            .edit()
            .tap_flush(|tx| {
                tx.remove_e(EId(0)); // op 0
                tx.insert_out(a, EId(0)); // op 1 — names the removed member
            })
            .expect_err("insert dangler");
        assert_eq!(
            err.violations,
            vec![Violation::DanglingEndpoint {
                op: 1,
                record: RecordDesc::Staged(1),
                endpoint_arena: "e",
                endpoint: 0,
            }]
        );

        // An out-of-range raw id is an UnknownOldId, never an index panic.
        let err = base
            .edit()
            .tap_flush(|tx| tx.insert_out(a, EId(99)))
            .expect_err("oob insert");
        assert_eq!(
            err.violations,
            vec![Violation::UnknownOldId { op: 0, arena: "e", id: 99 }]
        );
    }
});

// ---- root-sourced list: the head is a store field, one chain -----------

mod root_links {
    use metagraph::implement;
    use metagraph::stdlib::OutEdges;

    implement! {
        m2_fixtures::neg::RootMany as RootLinks {
            cs: intrusive_list,
        }
    }

    pub(super) fn walked(s: &RootLinks) -> Vec<u32> {
        s.g().out_edges().map(|c| c.id().0).collect()
    }
}

with_tap!(root_links, root_links_tests, {
    use super::root_links::RootLinks;

    #[test]
    fn root_headed_single_chain_carry() {
        let mut base = RootLinks::new();
        let c0 = base.add_c();
        let c1 = base.add_c();
        let _c2 = base.add_c();
        let c3 = base.add_c();
        // One store-level chain; insertion is the self-less op.
        base.insert_cs(c1);
        base.insert_cs(c3);
        base.insert_cs(c0);
        // Chain iterates [c0, c3, c1]; c2 exists but was never listed.
        assert_eq!(walked(&base), vec![0, 3, 1]);

        // Remove the chain's middle element and add a new (unlisted) record:
        // the walk threads c0 -> c1 directly.
        let out = base
            .edit()
            .tap_flush(|tx| {
                tx.remove_c(CId(3));
                let _ = tx.add_c();
            })
            .expect("flush");

        assert_eq!(out.store.c_count(), 4); // c0,c1,c2 kept + one staged
        assert_eq!(out.report.c_map, vec![Some(0), Some(1), Some(2), None]);
        assert_eq!(walked(&out.store), vec![0, 1]);
        out.store.validate().expect("validate");
        // Old store untouched.
        assert_eq!(walked(&base), vec![0, 3, 1]);
    }

    #[test]
    fn root_headed_self_less_insert_through_tx() {
        // M4 step 6: at an unfused-root self the insert verb is
        // inherent-only (no trait impl — the canonical arity takes the
        // self ref, which does not exist there). The self-less Insert op
        // replays through the store-level hook after the carry.
        let mut base = RootLinks::new();
        let c0 = base.add_c();
        let _c1 = base.add_c();
        base.insert_cs(c0);

        let out = base
            .edit()
            .tap_flush(|tx| {
                let c2 = tx.add_c(); // born unlisted
                tx.insert_cs(c2); // self-less: no host
            })
            .expect("flush");
        // Carried [c0'], then the staged c2' push-fronts: [c2', c0'].
        assert_eq!(walked(&out.store), vec![2, 0]);
        out.store.validate().expect("validate");
    }
});
