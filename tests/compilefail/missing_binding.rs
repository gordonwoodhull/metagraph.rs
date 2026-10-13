// A `many` arrow with no binding: `arena` vs `intrusive_list` is an
// explicit choice with no safe default.
use metagraph::implement;

implement! {
    metagraph::stdlib::Digraph as GraphStore {
        nodes:     arena,
        in_edges:  intrusive_list,
    }
}

fn main() {}
