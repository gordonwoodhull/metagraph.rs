// A binding naming an arrow the pattern does not declare.
use metagraph_exper::implement;

implement! {
    metagraph_exper::stdlib::Digraph as GraphStore {
        nodes:     arena,
        out_edges: intrusive_list,
        in_edges:  intrusive_list,
        weights:   arena,
    }
}

fn main() {}
