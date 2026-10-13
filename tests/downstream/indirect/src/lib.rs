//! Indirect-dependency negative case (the declaring-crate resolver's one
//! constraint, plan "Open entities"): the implementing crate must depend
//! directly on every member pattern's declaring crate. Here the
//! fixtures-local `DocTags` rides through a re-export, so
//! `proc_macro_crate::crate_name("m2-fixtures")` fails and `implement!`
//! must reject with a clean message — not a generated-code error.
//! (`DocTags` instantiates a fixtures-local root on purpose: since the
//! M3 step-9 promotion the stdlib shapes resolve through the runtime,
//! which is a direct dependency here, so only a member naming its own
//! declaring crate still exercises the rule.)
//!
//! Feature-gated (`indirect`, off by default) so the workspace stays green;
//! exercise with `cargo build -p downstream-indirect --features indirect`.

#[cfg(feature = "indirect")]
mod must_fail {
    use metagraph::implement;

    mod comp {
        use metagraph::compose_pattern;

        compose_pattern! {
            Indirect {
                instances:
                    main: metagraph::stdlib::Digraph,
                    // Textually reachable through the re-export; the baked
                    // package is still "m2-fixtures", which is not a direct
                    // dependency of THIS crate.
                    sub:  downstream_reexporter::fixtures::local::DocTags;
                sub.Tag <-> main.Node;
            }
        }
    }

    implement! {
        comp::Indirect as Store {
            main.nodes:     arena,
            main.out_edges: intrusive_list,
            main.in_edges:  intrusive_list,
            sub.tags:       arena,
        }
    }
}
