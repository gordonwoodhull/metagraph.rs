// A `many` arrow bound to `field`: wrong cardinality.
use metagraph::implement;

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes: field,
    }
}

fn main() {}
