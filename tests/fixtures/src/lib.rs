//! M2 test fixtures: the `Subgraph` pattern and the single-arrow roots it
//! leans on, declared in a crate every integration test can depend on (each
//! `tests/*.rs` is its own crate, so patterns with local roots cannot live
//! in a test file — the declaring-crate resolver would resolve `crate::`
//! against the wrong crate).
//!
//! Root naming follows the plan: a root pattern takes its arrow's name with
//! no suffix where the name is free (`MainNode`/`MainEdge` — their entity
//! namesakes are open and emit nothing), and the `-Of` suffix where the bare
//! name is taken (`SubNodeOf`, `SubEdgeOf` beside the entities; `EdgesOf`
//! beside the stdlib root). The trait is arrow-named regardless, so
//! `SubNodeOf` owns `subnode<M>`.

/// The single-arrow roots `Subgraph` instantiates.
pub mod roots {
    use metagraph_exper::pattern;

    pattern! {
        MainNode {
            entities: root SubNode, MainNode;
            arrows { main_node: SubNode -> MainNode, one; }
        }
    }
    pattern! {
        MainEdge {
            entities: root SubEdge, MainEdge;
            arrows { main_edge: SubEdge -> MainEdge, one; }
        }
    }
    pattern! {
        EdgesOf {
            entities: root Subgraph, SubEdge;
            arrows { subedges: Subgraph -> SubEdge, many; }
        }
    }
    pattern! {
        SubNodeOf {
            entities: root Subgraph, SubNode, MainNode;
            arrows { subnode: Subgraph -> SubNode, key(MainNode); }
        }
    }
    pattern! {
        SubEdgeOf {
            entities: root Subgraph, SubEdge, MainEdge;
            arrows { subedge: Subgraph -> SubEdge, key(MainEdge); }
        }
    }
}

pub mod shapes {
    use metagraph_exper::pattern;

    // The full Subgraph pattern (keyed arrows included). Compositions bind
    // `subnode`/`subedge` to `hash_index`.
    pattern! {
        Subgraph {
            entities: root Subgraph, SubNode, SubEdge, MainNode(open), MainEdge(open);
            arrows {
                subnodes:  metagraph_exper::stdlib::Nodes, Subgraph -> SubNode;
                subedges:  crate::roots::EdgesOf,              Subgraph -> SubEdge;
                main_node: crate::roots::MainNode,             SubNode -> MainNode;
                main_edge: crate::roots::MainEdge,             SubEdge -> MainEdge;
                subnode:   crate::roots::SubNodeOf,            Subgraph -> SubNode, key(MainNode);
                subedge:   crate::roots::SubEdgeOf,            Subgraph -> SubEdge, key(MainEdge);
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
                subedges:  crate::roots::EdgesOf,            Subgraph -> SubEdge;
                main_node: crate::roots::MainNode,           SubNode -> MainNode;
                main_edge: crate::roots::MainEdge,           SubEdge -> MainEdge;
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
            arrows { k: crate::roots::SubNodeOf, G -> N; }
        }
    }
}
