// compose-time structural check (fusion_shapes): a root-marked entity
// owns no id type to unify, so `<->` never touches one.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Bad {
            instances:
                outer: metagraph_exper::stdlib::Digraph,
                inner: metagraph_exper::stdlib::Digraph;
            outer.Graph <-> inner.Node;
        }
    }
}

fn main() {}
