// A `many` arrow with no binding: `arena` vs `intrusive_list` is an
// explicit choice with no safe default.
use metagraph_exper::implement;

implement! {
    metagraph_exper::stdlib::Digraph as GraphStore {
        nodes:     arena,
        in_edges:  intrusive_list,
    }
}

fn main() {}
