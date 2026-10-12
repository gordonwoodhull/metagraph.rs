//! Indirect-dependency negative case (the declaring-crate resolver's one
//! constraint, plan "Open entities"): the implementing crate must depend
//! directly on every member pattern's declaring crate. Here `Subgraph`
//! rides through a re-export, so `proc_macro_crate::crate_name("m2-fixtures")`
//! fails and `implement!` must reject with a clean message — not a
//! generated-code error.
//!
//! Feature-gated (`indirect`, off by default) so the workspace stays green;
//! exercise with `cargo build -p downstream-indirect --features indirect`.

#[cfg(feature = "indirect")]
mod must_fail {
    use metagraph_exper::implement;

    mod comp {
        use metagraph_exper::compose_pattern;

        compose_pattern! {
            Indirect {
                instances:
                    main: metagraph_exper::stdlib::Digraph,
                    // Textually reachable through the re-export; the baked
                    // package is still "m2-fixtures", which is not a direct
                    // dependency of THIS crate.
                    sub:  downstream_reexporter::fixtures::shapes::Subgraph;
                sub.MainNode <-> main.Node;
                sub.MainEdge <-> main.Edge;
            }
        }
    }

    implement! {
        comp::Indirect as Store {
            main.nodes:     arena,
            main.out_edges: intrusive_list,
            main.in_edges:  intrusive_list,
            sub.subnodes:   arena,
            sub.subedges:   arena,
            sub.subnode:    hash_index,
            sub.subedge:    hash_index,
        }
    }
}
