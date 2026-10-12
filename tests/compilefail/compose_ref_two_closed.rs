// compose-time structural check (fusion_shapes): `<->` pairs exactly one
// open entity with one closed entity — two closed entities have nothing
// to unify.
mod comp {
    use metagraph_exper::compose_pattern;

    compose_pattern! {
        Bad {
            instances:
                outer: metagraph_exper::stdlib::Digraph,
                inner: metagraph_exper::stdlib::Digraph;
            outer.Node <-> inner.Node;
        }
    }
}

fn main() {}
