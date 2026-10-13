// compose-time structural check (fusion_shapes): `<->` pairs exactly one
// open entity with one closed entity — two closed entities have nothing
// to unify.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Bad {
            instances:
                outer: metagraph::stdlib::Digraph,
                inner: metagraph::stdlib::Digraph;
            outer.Node <-> inner.Node;
        }
    }
}

fn main() {}
