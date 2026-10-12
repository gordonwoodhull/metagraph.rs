// implement!-time: a pattern with open entities cannot be implemented
// flat — open entities are closed by `compose_pattern!`, not `implement!`.
use metagraph_exper::implement;

implement! {
    m2_fixtures::shapes::Subgraph as SubgraphStore {
        subnodes:  arena,
        subedges:  arena,
        main_node: field,
        main_edge: field,
        subnode:   hash_index,
        subedge:   hash_index,
    }
}

fn main() {}
