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
}
