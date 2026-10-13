// The same arrow bound twice.
use metagraph::implement;

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        nodes:     intrusive_list,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
    }
}

fn main() {}
