// A `many` arrow bound to `field`: wrong cardinality.
use metagraph_exper::implement;

implement! {
    metagraph_exper::stdlib::Digraph as GraphStore {
        nodes: field,
    }
}

fn main() {}
