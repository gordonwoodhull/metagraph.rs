// compose-time docking check, same-instance arm: two elements of ONE
// instance fused by `<=>` — sort-sharing within an instance, defined by
// nothing (a flat schema over shared sorts lives inside one `pattern!`
// block). The arm was pinned at the M3 round-4 merge; this fixture rode
// with its implementation.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Same {
            instances: d: metagraph_exper::stdlib::Digraph;
            d.Node <=> d.Edge;
        }
    }
}

fn main() {}
