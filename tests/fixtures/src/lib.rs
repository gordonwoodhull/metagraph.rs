//! M2 test fixtures: the `Subgraph` shapes, declared in a crate every
//! integration test can depend on (each `tests/*.rs` is its own crate, so
//! patterns with local entities cannot live in a test file — the
//! declaring-crate resolver would resolve `crate::` against the wrong
//! crate).
//!
//! The single-arrow roots these shapes instantiate were M2's discoveries
//! and now live in the runtime stdlib (promoted in M3 step 9):
//! `metagraph_exper::stdlib::{MainNode, MainEdge, EdgesOf, SubNodeOf,
//! SubEdgeOf}`. Naming follows the resolved rule there — a root pattern
//! takes its arrow's name with no suffix where the name is free
//! (`MainNode`/`MainEdge`), the `-Of` suffix where the bare name is taken
//! (`SubNodeOf`/`SubEdgeOf`/`EdgesOf`); the trait is arrow-named
//! regardless, so `SubNodeOf` owns `subnode<M>`.

pub mod shapes {
    use metagraph_exper::pattern;

    // The full Subgraph pattern (keyed arrows included). Compositions bind
    // `subnode`/`subedge` to `hash_index`.
    pattern! {
        Subgraph {
            entities: root Subgraph, SubNode, SubEdge, MainNode(open), MainEdge(open);
            arrows {
                subnodes:  metagraph_exper::stdlib::Nodes, Subgraph -> SubNode;
                subedges:  metagraph_exper::stdlib::EdgesOf,              Subgraph -> SubEdge;
                main_node: metagraph_exper::stdlib::MainNode,             SubNode -> MainNode;
                main_edge: metagraph_exper::stdlib::MainEdge,             SubEdge -> MainEdge;
                subnode:   metagraph_exper::stdlib::SubNodeOf,            Subgraph -> SubNode, key(MainNode);
                subedge:   metagraph_exper::stdlib::SubEdgeOf,            Subgraph -> SubEdge, key(MainEdge);
            }
        }
    }

    // The keyed-arrow-free variant, for compositions that exercise fusion
    // codegen before `hash_index` binds (M2 step 7's Hierarchy test).
    pattern! {
        SubPlain {
            entities: root Subgraph, SubNode, SubEdge, MainNode(open), MainEdge(open);
            arrows {
                subnodes:  metagraph_exper::stdlib::Nodes, Subgraph -> SubNode;
                subedges:  metagraph_exper::stdlib::EdgesOf,            Subgraph -> SubEdge;
                main_node: metagraph_exper::stdlib::MainNode,           SubNode -> MainNode;
                main_edge: metagraph_exper::stdlib::MainEdge,           SubEdge -> MainEdge;
            }
        }
    }

    // The Chain pattern for edge-chain refinement (M2 step 11): an outer
    // edge is-also a chain of inner edges. `Elem` is open — there is no Elem
    // arena; chain membership threads through the inner-edge records
    // themselves (a head field on the fused outer-edge record, a `next` link
    // on each inner edge), exactly the way `out_edges` threads through edge
    // records.
    pattern! {
        Chain {
            entities: root Chain, Elem(open);
            arrows { elems: metagraph_exper::stdlib::OutEdges, Chain -> Elem; }
        }
    }

    // A keyed pattern with no open entities: drivable by a flat `implement!`
    // (M3 step 5's plain-self keyed placement — `tags` is keyed at an
    // element self; `marks` rides the root, the store-level placement).
    pattern! {
        KeyedPlain {
            entities: root G, N, M;
            arrows {
                nodes: metagraph_exper::stdlib::Nodes, G -> N;
                tags:  metagraph_exper::stdlib::SubNodeOf,        N -> M, key(N);
                marks: metagraph_exper::stdlib::SubNodeOf,        G -> M, key(M);
            }
        }
    }

    // The CSR driver's primitive shape: `out` is a primitive `many` arrow
    // sourced at the element `N` (no `by` — inserts ride the public insert
    // op and `on_add` grows the buckets), unlike the stdlib `Digraph`'s
    // by-derived `out_edges`. M3 step 8's flat phased suite binds `out` to
    // `csr_adjacency`.
    pattern! {
        CsrDigraph {
            entities: root G, N, E;
            arrows {
                nodes:  metagraph_exper::stdlib::Nodes, G -> N;
                out:    metagraph_exper::stdlib::OutEdges, N -> E;
                source: metagraph_exper::stdlib::Source,  E -> N;
                target: metagraph_exper::stdlib::Target,  E -> N;
            }
        }
    }
}

/// Minimal shapes for the M2 step-13 negative tests: patterns that exist
/// so implement!-time checks have small legal compositions to reject.
pub mod neg {
    use metagraph_exper::pattern;

    // `pin` is a `one` arrow into the closed element `P`: under
    // `outer.Node <=> pinned.G` its field would have to hold an embedded
    // (host-anchored) id, which a constructor argument cannot supply.
    pattern! {
        Pinned {
            entities: root G, N, P;
            arrows {
                g:   metagraph_exper::stdlib::Nodes,  G -> N;
                pin: metagraph_exper::stdlib::Source, N -> P;
            }
        }
    }

    // `cs` is a `many` arrow sourced at the closed element `N`: under
    // `outer.Node <=> chained.G` its intrusive head would live on an
    // embedded record (the frontier limit).
    pattern! {
        Chained {
            entities: root G, N, C;
            arrows {
                g:  metagraph_exper::stdlib::Nodes,    G -> N;
                cs: metagraph_exper::stdlib::OutEdges, N -> C;
            }
        }
    }

    // Two primitive `many` arrows targeting one entity: binding both to
    // `arena` is the duplicate existence-binding rejection (M3 step 7);
    // binding the element-sourced one alone is the flat `hosts_arena`
    // rejection.
    pattern! {
        TwoInto {
            entities: root G, N, M;
            arrows {
                m1: metagraph_exper::stdlib::Nodes, G -> N;
                m2: metagraph_exper::stdlib::Edges, M -> N;
            }
        }
    }

    // `r` is a `one` arrow sourced at the pattern's root: unfused, the
    // field backing has no self record to live on.
    pattern! {
        RootSourced {
            entities: root G, P;
            arrows { r: metagraph_exper::stdlib::Source, G -> P; }
        }
    }

    // Instantiated under names `p` and `p_q`, its two element-sourced
    // arrows make the store method `insert_p_q_r` ambiguous (instance p's
    // arrow `q_r` vs instance p_q's arrow `r`) — the generated-name
    // collision check.
    pattern! {
        Collide {
            entities: root G, A, B, X;
            arrows {
                m:   metagraph_exper::stdlib::Nodes,  G -> X;
                q_r: metagraph_exper::stdlib::OutEdges, A -> B;
                r:   metagraph_exper::stdlib::Edges,    A -> X;
            }
        }
    }

    // Instantiates the keyed root `SubNodeOf` without binding a key sort.
    pattern! {
        Keyless {
            entities: root G, N, M;
            arrows { k: metagraph_exper::stdlib::SubNodeOf, G -> N; }
        }
    }

    // A keyed arrow whose target is open — closable by `<->` onto another
    // instance's *embedded* class while the self stays an unfused root: the
    // value-host-agreement check has no self-side anchor of the host id
    // type there (M3 commit 3's recorded gap).
    pattern! {
        AnchoredKey {
            entities: root G, E(open), M;
            arrows { k: metagraph_exper::stdlib::SubNodeOf, G -> E, key(M); }
        }
    }

    // A `many` arrow sourced at the pattern's own root (unfused under a
    // composition): the relaxed store-level intrusive placement — the head
    // lives on the store, the hook is self-less, the read rides the root
    // handle.
    pattern! {
        RootMany {
            entities: root G, C;
            arrows { cs: metagraph_exper::stdlib::OutEdges, G -> C; }
        }
    }
}

/// A fixtures-local root and one shape instantiating it. After the M3
/// step-9 promotion every root the `shapes` module instantiates lives in
/// the runtime stdlib — which every implement site already depends on
/// directly — so those shapes can no longer exercise the declaring-crate
/// resolver's one rejection. `DocTags` still names its own declaring
/// crate (`crate::local::Tags`), keeping `downstream-indirect`'s negative
/// case armed: reaching it only through a re-export must stay a clean
/// compile error there.
pub mod local {
    use metagraph_exper::pattern;

    pattern! {
        Tags {
            entities: root Doc, Tag;
            arrows { tags: Doc -> Tag, many; }
        }
    }

    pattern! {
        DocTags {
            entities: root Doc, Tag(open);
            arrows { tags: crate::local::Tags, Doc -> Tag; }
        }
    }
}
