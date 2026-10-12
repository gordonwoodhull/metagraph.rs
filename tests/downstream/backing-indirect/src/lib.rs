//! Indirect-dependency negative case for a *backing* courier (M3 step 7's
//! re-export routing fixture): the implementing crate must depend directly
//! on the backing's declaring crate. Here `counting_list` rides through a
//! re-export, so `proc_macro_crate::crate_name("backing-provider")` fails
//! and `implement!` must reject with the resolver's clean message — not a
//! generated-code error.
//!
//! Feature-gated (`indirect`, off by default) so the workspace stays green;
//! exercise with `cargo build -p downstream-backing-indirect --features indirect`.

#[cfg(feature = "indirect")]
mod must_fail {
    use metagraph_exper::implement;

    mod shapes {
        use metagraph_exper::pattern;

        pattern! {
            Tagged {
                entities: root Doc, Tag, Link;
                arrows {
                    tags:  metagraph_exper::stdlib::Nodes,    Doc -> Tag;
                    links: metagraph_exper::stdlib::OutEdges, Tag -> Link;
                }
            }
        }
    }

    implement! {
        crate::must_fail::shapes::Tagged as TaggedStore {
            tags:  arena,
            // Textually reachable through the re-export; the baked package
            // is still "backing-provider", which is not a direct dependency
            // of THIS crate.
            links: downstream_reexporter::backing_prov::lists::counting_list,
        }
    }
}
