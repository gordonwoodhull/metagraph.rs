// implement!-time home check: an `intrusive_list` arrow sourced at an
// embedded class would put its head field on records reached only
// through a host path (the frontier limit).
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        ChainedComp {
            instances:
                outer:  metagraph_exper::stdlib::Digraph,
                chained: m2_fixtures::neg::Chained;
            outer.Node <=> chained.G;
        }
    }
}

use metagraph_exper::implement;

implement! {
    comp::ChainedComp as ChainedStore {
        outer.nodes:     arena,
        outer.out_edges: intrusive_list,
        outer.in_edges:  intrusive_list,
        chained.g:       arena,
        chained.cs:      intrusive_list,
    }
}

fn main() {}
