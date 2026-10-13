// A binding naming an arrow the pattern does not declare.
use metagraph::implement;

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
        weights:   arena,
    }
}

fn main() {}
