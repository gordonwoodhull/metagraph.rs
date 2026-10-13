// compose-time structural check (open_entities_closed_once): closing one
// open entity twice would unify two closed id types through it.
mod comp {
    use metagraph::compose_pattern;

    compose_pattern! {
        Bad {
            instances:
                outer: metagraph::stdlib::Digraph,
                inner: metagraph::stdlib::Digraph,
                sub:   m2_fixtures::shapes::Subgraph;
            sub.MainNode <-> outer.Node;
            sub.MainNode <-> inner.Node;
        }
    }
}

fn main() {}
