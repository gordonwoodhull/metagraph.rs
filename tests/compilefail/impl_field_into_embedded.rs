// implement!-time home check: a `field`-backed `one` arrow into an
// embedded class would have to hold a host-anchored path id, which a
// constructor argument cannot supply.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        PinnedComp {
            instances:
                outer:  metagraph_exper::stdlib::Digraph,
                pinned: m2_fixtures::neg::Pinned;
            outer.Node <=> pinned.G;
        }
    }
}

use metagraph_exper::implement;

implement! {
    comp::PinnedComp as PinnedStore {
        outer.nodes:     arena,
        outer.out_edges: intrusive_list,
        outer.in_edges:  intrusive_list,
        pinned.g:        arena,
        // `pin` is a `one` arrow, so `field` is the default backing —
        // into the embedded class `P`.
    }
}

fn main() {}
