// An `one` arrow bound to `intrusive_list`: wrong cardinality, the other
// direction of the check.
use metagraph::implement;

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
        source:   intrusive_list,
    }
}

fn main() {}
